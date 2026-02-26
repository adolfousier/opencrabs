//! Agent Service Implementation
//!
//! Core service for managing AI agent conversations, coordinating between
//! LLM providers, context management, and data persistence.

use super::context::AgentContext;
use super::error::{AgentError, Result};
use crate::brain::provider::{
    ContentBlock, ImageSource, LLMRequest, LLMResponse, Message, Provider, ProviderStream, Role,
    StopReason,
};
use crate::brain::tools::{ToolExecutionContext, ToolRegistry};
use crate::services::{MessageService, ServiceContext, SessionService};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Tool approval request information
#[derive(Debug, Clone)]
pub struct ToolApprovalInfo {
    /// Tool name
    pub tool_name: String,
    /// Tool description
    pub tool_description: String,
    /// Tool input parameters
    pub tool_input: Value,
    /// Tool capabilities
    pub capabilities: Vec<String>,
}

/// Type alias for approval callback function
/// Returns true if approved, false if denied
pub type ApprovalCallback = Arc<
    dyn Fn(ToolApprovalInfo) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> + Send + Sync,
>;

/// Progress event emitted during tool execution
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Thinking,
    ToolStarted {
        tool_name: String,
        tool_input: Value,
    },
    ToolCompleted {
        tool_name: String,
        tool_input: Value,
        success: bool,
        summary: String,
    },
    /// Intermediate text the agent sends between tool call batches
    IntermediateText {
        text: String,
    },
    /// Real-time streaming chunk from the LLM (word-by-word)
    StreamingChunk {
        text: String,
    },
    Compacting,
    /// Compaction finished — carry the summary so the TUI can display it
    CompactionSummary {
        summary: String,
    },
    /// Build completed — TUI should offer restart
    RestartReady {
        status: String,
    },
    /// Real-time token count update — fire after every API response and tool execution
    TokenCount(usize),
    //    /// A queued user message was injected into the agent context between tool iterations
    //    QueuedMessageInjected { content: String },
}

/// Callback for reporting progress during agent execution
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// Callback for requesting sudo password from the user.
/// Takes the command string, returns Ok(Some(password)) or Ok(None) if cancelled.
pub type SudoCallback = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send>> + Send + Sync,
>;

/// Callback for checking if a user message has been queued during tool execution.
/// Returns Some(message) if a message is waiting, None otherwise. Must not block.
pub type MessageQueueCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

/// Agent Service for managing AI conversations
pub struct AgentService {
    /// LLM provider
    provider: Arc<dyn Provider>,

    /// Service context for database operations
    context: ServiceContext,

    /// Tool registry for executing tools
    tool_registry: Arc<ToolRegistry>,

    /// Maximum tool execution iterations (0 = unlimited, relies on loop detection)
    max_tool_iterations: usize,

    /// System brain template
    default_system_brain: Option<String>,

    /// Whether to auto-approve tool execution
    auto_approve_tools: bool,

    /// Context window limit in tokens from config
    context_limit: u32,

    /// Max output tokens for API calls from config
    max_tokens: u32,

    /// Callback for requesting tool approval from user
    approval_callback: Option<ApprovalCallback>,

    /// Callback for reporting progress during tool execution
    progress_callback: Option<ProgressCallback>,

    /// Callback for checking queued user messages between tool iterations
    message_queue_callback: Option<MessageQueueCallback>,

    /// Callback for requesting sudo password from user
    sudo_callback: Option<SudoCallback>,

    /// Working directory for tool execution (shared, mutable at runtime via /cd or agent NLP)
    working_directory: Arc<std::sync::RwLock<std::path::PathBuf>>,

    /// Brain path (~/.opencrabs/) for loading brain files
    brain_path: Option<std::path::PathBuf>,
}

impl AgentService {
    /// Create a new agent service
    pub fn new(provider: Arc<dyn Provider>, context: ServiceContext) -> Self {
        let config = crate::config::Config::load().unwrap_or_default();

        Self {
            provider,
            context,
            tool_registry: Arc::new(ToolRegistry::new()),
            max_tool_iterations: 0, // 0 = unlimited (loop detection is the safety net)
            default_system_brain: None,
            auto_approve_tools: false,
            context_limit: config.agent.context_limit,
            max_tokens: config.agent.max_tokens,
            approval_callback: None,
            progress_callback: None,
            message_queue_callback: None,
            sudo_callback: None,
            working_directory: Arc::new(std::sync::RwLock::new(
                std::env::current_dir().unwrap_or_default(),
            )),
            brain_path: None,
        }
    }

    /// Get the service context
    pub fn context(&self) -> &ServiceContext {
        &self.context
    }

    /// Get context limit from config
    pub fn context_limit(&self) -> u32 {
        self.context_limit
    }

    /// Get max tokens from config
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    /// Get the tool registry
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    /// Set the default system brain
    pub fn with_system_brain(mut self, prompt: String) -> Self {
        self.default_system_brain = Some(prompt);
        self
    }

    /// Set maximum tool iterations
    pub fn with_max_tool_iterations(mut self, max: usize) -> Self {
        self.max_tool_iterations = max;
        self
    }

    /// Set the tool registry
    pub fn with_tool_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = registry;
        self
    }

    /// Set whether to auto-approve tool execution
    pub fn with_auto_approve_tools(mut self, auto_approve: bool) -> Self {
        self.auto_approve_tools = auto_approve;
        self
    }

    /// Set the approval callback for interactive tool approval
    pub fn with_approval_callback(mut self, callback: Option<ApprovalCallback>) -> Self {
        self.approval_callback = callback;
        self
    }

    /// Set the progress callback for reporting tool execution progress
    pub fn with_progress_callback(mut self, callback: Option<ProgressCallback>) -> Self {
        self.progress_callback = callback;
        self
    }

    /// Set the message queue callback for injecting user messages between tool iterations
    pub fn with_message_queue_callback(mut self, callback: Option<MessageQueueCallback>) -> Self {
        self.message_queue_callback = callback;
        self
    }

    /// Set the sudo password callback for interactive sudo prompts
    pub fn with_sudo_callback(mut self, callback: Option<SudoCallback>) -> Self {
        self.sudo_callback = callback;
        self
    }

    /// Set the working directory for tool execution
    pub fn with_working_directory(self, working_directory: std::path::PathBuf) -> Self {
        *self
            .working_directory
            .write()
            .expect("working_directory lock poisoned") = working_directory;
        self
    }

    /// Get the current working directory
    pub fn working_directory(&self) -> std::path::PathBuf {
        self.working_directory
            .read()
            .expect("working_directory lock poisoned")
            .clone()
    }

    /// Change the working directory at runtime (called from /cd or agent tools)
    pub fn set_working_directory(&self, path: std::path::PathBuf) {
        *self
            .working_directory
            .write()
            .expect("working_directory lock poisoned") = path;
    }

    /// Get a shared handle to the working directory (for tools that need to mutate it)
    pub fn shared_working_directory(&self) -> Arc<std::sync::RwLock<std::path::PathBuf>> {
        Arc::clone(&self.working_directory)
    }

    /// Set the brain path (~/.opencrabs/)
    pub fn with_brain_path(mut self, brain_path: std::path::PathBuf) -> Self {
        self.brain_path = Some(brain_path);
        self
    }

    /// Get the provider name
    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    /// Get the system brain
    pub fn system_brain(&self) -> Option<&String> {
        self.default_system_brain.as_ref()
    }

    /// Get the default model for this provider
    pub fn provider_model(&self) -> &str {
        self.provider.default_model()
    }

    /// Get the list of supported models for this provider (hardcoded fallback)
    pub fn supported_models(&self) -> Vec<String> {
        self.provider.supported_models()
    }

    /// Fetch available models from the provider API (live)
    pub async fn fetch_models(&self) -> Vec<String> {
        self.provider.fetch_models().await
    }

    /// Get a reference to the underlying LLM provider
    pub fn provider(&self) -> &Arc<dyn Provider> {
        &self.provider
    }

    /// Get context window size for a given model
    pub fn context_window_for_model(&self, _model: &str) -> u32 {
        self.context_limit
    }

    /// Send a message and get a response
    ///
    /// This will:
    /// 1. Load conversation context from the database
    /// 2. Add the new user message
    /// 3. Send to the LLM provider
    /// 4. Save the response to the database
    /// 5. Update token usage
    pub async fn send_message(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<AgentResponse> {
        // Prepare message context (common setup logic)
        let (_model_name, request, message_service, session_service) = self
            .prepare_message_context(session_id, user_message, model)
            .await?;

        // Send to provider
        let response = self
            .provider
            .complete(request)
            .await
            .map_err(AgentError::Provider)?;

        // Extract text from response
        let assistant_text = Self::extract_text_from_response(&response);

        // Save assistant response to database
        let assistant_db_msg = message_service
            .create_message(session_id, "assistant".to_string(), assistant_text.clone())
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Calculate total tokens and cost for this message
        let total_tokens = response.usage.input_tokens + response.usage.output_tokens;
        let cost = self.provider.calculate_cost(
            &response.model,
            response.usage.input_tokens,
            response.usage.output_tokens,
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

        Ok(AgentResponse {
            message_id: assistant_db_msg.id,
            content: assistant_text,
            stop_reason: response.stop_reason,
            context_tokens: response.usage.input_tokens,
            usage: response.usage,
            cost,
            model: response.model,
        })
    }

    /// Send a message and get a streaming response
    ///
    /// Returns a stream of response chunks that can be consumed incrementally.
    pub async fn send_message_streaming(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<AgentStreamResponse> {
        // Prepare message context (common setup logic)
        let (model_name, request, _message_service, _session_service) = self
            .prepare_message_context(session_id, user_message, model)
            .await?;

        // Add streaming flag to request
        let request = request.with_streaming();

        // Get streaming response from provider
        let stream = self
            .provider
            .stream(request)
            .await
            .map_err(AgentError::Provider)?;

        Ok(AgentStreamResponse {
            session_id,
            message_id: Uuid::new_v4(),
            stream,
            model: model_name,
        })
    }

    /// Send a message with automatic tool execution
    ///
    /// This method implements a tool execution loop:
    /// 1. Send message to LLM
    /// 2. If LLM requests tool use, execute the tool
    /// 3. Send tool results back to LLM
    /// 4. Repeat until LLM finishes or max iterations reached
    pub async fn send_message_with_tools(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<AgentResponse> {
        self.send_message_with_tools_and_mode(session_id, user_message, model, false, None)
            .await
    }

    /// Send a message with automatic tool execution and explicit read-only mode control
    pub async fn send_message_with_tools_and_mode(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
        read_only_mode: bool,
        cancel_token: Option<CancellationToken>,
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

        let model_name = model.unwrap_or_else(|| self.provider.default_model().to_string());
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

        // Build user message — detect and attach images from paths/URLs
        let user_msg = Self::build_user_message(&user_message).await;
        context.add_message(user_msg);

        // Save user message to database (text only — images are ephemeral)
        let _user_db_msg = message_service
            .create_message(session_id, "user".to_string(), user_message)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Reserve tokens for tool definitions (not tracked in context.token_count).
        // Each tool schema is roughly 300-800 tokens; reserve a flat budget.
        let tool_overhead = self.tool_registry.count() * 500;
        let effective_max = context.max_tokens.saturating_sub(tool_overhead);
        let effective_usage = if effective_max > 0 {
            (context.token_count as f64 / effective_max as f64) * 100.0
        } else {
            100.0
        };

        // Auto-compaction: if context usage exceeds 80% (accounting for tool overhead)
        if effective_usage > 80.0 {
            tracing::warn!(
                "Context usage at {:.0}% (effective {:.0}% with {} tool overhead) — triggering auto-compaction",
                context.usage_percentage(),
                effective_usage,
                tool_overhead,
            );
            match self.compact_context(&mut context, &model_name).await {
                Ok(summary) => {
                    // Stream the summary to chat so user can see it
                    if let Some(ref cb) = self.progress_callback {
                        cb(ProgressEvent::CompactionSummary { summary });
                    }
                    let mut cont_text =
                        "[SYSTEM: Context was auto-compacted. The summary above has full context. \
                         Continue the task immediately using tools. Do NOT repeat completed work. \
                         Do NOT ask for instructions.]"
                            .to_string();
                    if !self.auto_approve_tools {
                        cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
                    }
                    context.add_message(Message::user(cont_text));
                }
                Err(e) => {
                    tracing::error!("Pre-loop compaction failed: {}", e);
                    if let Some(ref cb) = self.progress_callback {
                        cb(ProgressEvent::IntermediateText {
                            text: format!(
                                "Context compaction failed: {}. Continuing with current context.",
                                e
                            ),
                        });
                    }
                }
            }
        }

        // Hard safety net: if context is above 90%, force-truncate old messages
        // regardless of compaction success. Never let it blow past the window.
        let post_usage = if effective_max > 0 {
            (context.token_count as f64 / effective_max as f64) * 100.0
        } else {
            100.0
        };
        if post_usage > 90.0 {
            tracing::warn!(
                "Context still at {:.0}% after compaction attempt — hard-truncating to fit",
                post_usage
            );
            let target_tokens = (effective_max as f64 * 0.6) as usize; // truncate down to 60%
            context.trim_to_target(target_tokens);
        }

        // Create tool execution context
        let mut tool_context = ToolExecutionContext::new(session_id)
            .with_auto_approve(self.auto_approve_tools)
            .with_working_directory(
                self.working_directory
                    .read()
                    .expect("working_directory lock poisoned")
                    .clone(),
            )
            .with_read_only_mode(read_only_mode);
        tool_context.sudo_callback = self.sudo_callback.clone();
        tool_context.shared_working_directory = Some(Arc::clone(&self.working_directory));

        // Tool execution loop
        let mut iteration = 0;
        let mut total_input_tokens = 0u32;
        let mut total_output_tokens = 0u32;
        let mut last_input_tokens = 0u32;
        let mut final_response: Option<LLMResponse> = None;
        let mut accumulated_text = String::new(); // Collect text from all iterations (not just final)
        let mut recent_tool_calls: Vec<String> = Vec::new(); // Track tool calls to detect loops
        let mut loop_break_reason: Option<String> = None; // Why the loop broke (if not normal exit)
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
                break;
            }

            iteration += 1;

            // Emit thinking progress
            if let Some(ref cb) = self.progress_callback {
                cb(ProgressEvent::Thinking);
            }

            // --- HARD CONTEXT BUDGET CHECK (every iteration) ---
            // Tool results accumulate inside the loop. Re-check context budget
            // BEFORE every API call to prevent sending >200K tokens (which
            // Anthropic will happily bill for without rejection).
            let tool_overhead = self.tool_registry.count() * 500;
            let effective_max = context.max_tokens.saturating_sub(tool_overhead);
            let effective_usage = if effective_max > 0 {
                (context.token_count as f64 / effective_max as f64) * 100.0
            } else {
                100.0
            };
            if effective_usage > 80.0 {
                tracing::warn!(
                    "Context at {:.0}% inside tool loop (iteration {}) — compacting before next API call",
                    effective_usage,
                    iteration,
                );
                match self.compact_context(&mut context, &model_name).await {
                    Ok(summary) => {
                        // Stream the summary to chat so user can see it
                        if let Some(ref cb) = self.progress_callback {
                            cb(ProgressEvent::CompactionSummary { summary });
                        }
                        // Inject continuation — no "acknowledge first", go straight to tools
                        let mut cont_text = "[SYSTEM: Context was auto-compacted. The summary above has full context. \
                             Continue the task immediately using tools. Do NOT repeat completed work. \
                             Do NOT ask for instructions.]".to_string();
                        if !self.auto_approve_tools {
                            cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
                        }
                        context.add_message(Message::user(cont_text));
                    }
                    Err(e) => {
                        tracing::error!("Pre-API-call compaction failed: {}", e);
                        loop_break_reason = Some(format!("Context compaction failed: {}", e));
                        break;
                    }
                }
            }

            // Hard safety net: if context is above 90% even after compaction, force-truncate
            let post_loop_usage = if effective_max > 0 {
                (context.token_count as f64 / effective_max as f64) * 100.0
            } else {
                100.0
            };
            if post_loop_usage > 90.0 {
                tracing::warn!(
                    "Context at {:.0}% in tool loop after compaction — hard-truncating",
                    post_loop_usage
                );
                let target = (effective_max as f64 * 0.6) as usize;
                context.trim_to_target(target);
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
            let response = match self.stream_complete(request, cancel_token.as_ref()).await {
                Ok(resp) => resp,
                Err(ref e)
                    if e.to_string().contains("prompt is too long")
                        || e.to_string().contains("too many tokens") =>
                {
                    tracing::warn!("Prompt too long for provider — emergency compaction");
                    let err_msg = e.to_string();
                    match self.compact_context(&mut context, &model_name).await {
                        Ok(_) => {
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
                    self.stream_complete(retry_req, cancel_token.as_ref())
                        .await
                        .map_err(AgentError::Provider)?
                }
                Err(e) => return Err(AgentError::Provider(e)),
            };

            // Track token usage — fall back to tiktoken estimate when provider
            // doesn't report usage (e.g. MiniMax streaming ignores include_usage)
            last_input_tokens = if response.usage.input_tokens > 0 {
                response.usage.input_tokens
            } else {
                tracing::debug!(
                    "Provider reported 0 input tokens, using tiktoken estimate: {}",
                    context.token_count
                );
                context.token_count as u32
            };
            total_input_tokens += last_input_tokens;
            total_output_tokens += response.usage.output_tokens;

            // Calibrate context token count with the API's real input_tokens.
            // Even with tiktoken, there's some drift since Anthropic's tokenizer differs slightly.
            // The API knows the exact count — use it to keep our tracking honest.
            let api_input = response.usage.input_tokens as usize;
            let tool_overhead = self.tool_registry.count() * 500;
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
            if let Some(ref cb) = self.progress_callback {
                cb(ProgressEvent::TokenCount(context.token_count));
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
            tracing::debug!("Response raw: {:?}", response);
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
                                "[TOOL_EXEC] ⚠️ Tool '{}' received EMPTY input! This will fail. Full input: {:?}",
                                name,
                                input
                            );
                        }

                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {
                        tracing::debug!("Block {}: Other content block", i);
                    }
                }
            }

            // Accumulate text from every iteration
            if !iteration_text.is_empty() {
                if !accumulated_text.is_empty() {
                    accumulated_text.push_str("\n\n");
                }
                accumulated_text.push_str(&iteration_text);
            }

            tracing::debug!("Found {} tool uses to execute", tool_uses.len());

            if tool_uses.is_empty() {
                if iteration > 0 {
                    tracing::info!("Agent completed after {} tool iterations", iteration);
                    // Emit final text so TUI persists it as a permanent message
                    if !iteration_text.is_empty() {
                        if let Some(ref cb) = self.progress_callback {
                            cb(ProgressEvent::IntermediateText {
                                text: iteration_text,
                            });
                        }
                    }
                } else {
                    tracing::info!("Agent responded with text only (no tool calls)");
                }
                final_response = Some(response);
                break;
            }

            // Emit intermediate text to TUI so it appears before the tool calls
            if !iteration_text.is_empty()
                && let Some(ref cb) = self.progress_callback
            {
                cb(ProgressEvent::IntermediateText {
                    text: iteration_text,
                });
            }

            // Detect tool loops: Track the current batch of tool calls
            // Include arguments in signature to distinguish different calls
            // For example: ls(./src) vs ls(./src/cli) should be different
            let current_call_signature = tool_uses
                .iter()
                .map(|(_, name, input)| {
                    match name.as_str() {
                        "plan" => {
                            // Extract operation from plan tool input
                            if let Some(operation) = input.get("operation").and_then(|v| v.as_str())
                            {
                                // For add_task, include task title to distinguish different tasks
                                if operation == "add_task" {
                                    if let Some(title) = input.get("title").and_then(|v| v.as_str())
                                    {
                                        format!("{}:{}:{}", name, operation, title)
                                    } else {
                                        format!("{}:{}", name, operation)
                                    }
                                } else {
                                    format!("{}:{}", name, operation)
                                }
                            } else {
                                name.to_string()
                            }
                        }

                        // File system exploration tools - include path to distinguish calls
                        "ls" => {
                            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                                // Normalize path separators for consistent comparison
                                let normalized = path.replace('\\', "/");
                                format!("ls:{}", normalized)
                            } else {
                                "ls:".to_string()
                            }
                        }

                        "glob" => {
                            if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
                                format!("glob:{}", pattern)
                            } else {
                                "glob:".to_string()
                            }
                        }

                        "grep" => {
                            // Include pattern AND path to distinguish searches
                            let pattern =
                                input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            format!("grep:{}:{}", pattern, path)
                        }

                        "read" => {
                            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                                let normalized = path.replace('\\', "/");
                                format!("read:{}", normalized)
                            } else {
                                "read:".to_string()
                            }
                        }

                        // File modification tools - include file path
                        "write" | "edit" => {
                            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                                let normalized = path.replace('\\', "/");
                                format!("{}:{}", name, normalized)
                            } else {
                                format!("{}:", name)
                            }
                        }

                        // Command execution - include command
                        "bash" => {
                            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                                // Normalize and truncate for signature
                                let cmd_normalized = cmd.replace('\\', "/");
                                let cmd_short: String = cmd_normalized.chars().take(100).collect();
                                format!("bash:{}", cmd_short)
                            } else {
                                "bash:".to_string()
                            }
                        }

                        // session_search: include operation + query to distinguish calls
                        "session_search" => {
                            let op = input
                                .get("operation")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
                            format!("session_search:{}:{}", op, query)
                        }

                        // web_search: include query to distinguish different searches
                        "web_search" => {
                            let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
                            format!("web_search:{}", query)
                        }

                        // http_request: include method + url to distinguish calls
                        "http_request" => {
                            let method = input.get("method").and_then(|v| v.as_str()).unwrap_or("");
                            let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
                            format!("http_request:{}:{}", method, url)
                        }

                        // Other tools: just use name
                        _ => name.to_string(),
                    }
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
                    break;
                }

                tracing::info!("Executing tool '{}' (iteration {})", tool_name, iteration,);

                // Save tool input for progress reporting (before it's moved to execute)
                let tool_input_for_progress = tool_input.clone();

                // Build short description for DB persistence
                tool_descriptions.push(Self::format_tool_summary(&tool_name, &tool_input));

                // Emit tool started progress
                if let Some(ref cb) = self.progress_callback {
                    cb(ProgressEvent::ToolStarted {
                        tool_name: tool_name.clone(),
                        tool_input: tool_input_for_progress.clone(),
                    });
                }

                // Check if approval is needed
                let needs_approval = if let Some(tool) = self.tool_registry.get(&tool_name) {
                    tool.requires_approval()
                        && !self.auto_approve_tools
                        && !tool_context.auto_approve
                } else {
                    false
                };

                // Request approval if needed
                if needs_approval {
                    if let Some(ref approval_callback) = self.approval_callback {
                        // Get tool details for approval request
                        let tool_info = if let Some(tool) = self.tool_registry.get(&tool_name) {
                            ToolApprovalInfo {
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
                        match approval_callback(tool_info).await {
                            Ok(approved) => {
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
                                tracing::info!("User approved tool '{}'", tool_name);
                                // Create approved context for this tool execution
                                let approved_tool_context = ToolExecutionContext {
                                    session_id: tool_context.session_id,
                                    working_directory: tool_context.working_directory.clone(),
                                    env_vars: tool_context.env_vars.clone(),
                                    auto_approve: true, // User approved this execution
                                    timeout_secs: tool_context.timeout_secs,
                                    read_only_mode: tool_context.read_only_mode,
                                    sudo_callback: tool_context.sudo_callback.clone(),
                                    shared_working_directory: tool_context
                                        .shared_working_directory
                                        .clone(),
                                };

                                // Execute the tool with approved context
                                match self
                                    .tool_registry
                                    .execute(&tool_name, tool_input, &approved_tool_context)
                                    .await
                                {
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
                                        if let Some(ref cb) = self.progress_callback {
                                            cb(ProgressEvent::ToolCompleted {
                                                tool_name: tool_name.clone(),
                                                tool_input: tool_input_for_progress.clone(),
                                                success,
                                                summary: output_summary,
                                            });
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
                                        if let Some(ref cb) = self.progress_callback {
                                            cb(ProgressEvent::ToolCompleted {
                                                tool_name: tool_name.clone(),
                                                tool_input: tool_input_for_progress.clone(),
                                                success: false,
                                                summary: output_summary,
                                            });
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

                // Execute the tool (no approval needed)
                match self
                    .tool_registry
                    .execute(&tool_name, tool_input, &tool_context)
                    .await
                {
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
                        if let Some(ref cb) = self.progress_callback {
                            cb(ProgressEvent::ToolCompleted {
                                tool_name: tool_name.clone(),
                                tool_input: tool_input_for_progress.clone(),
                                success,
                                summary: output_summary,
                            });
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
                        if let Some(ref cb) = self.progress_callback {
                            cb(ProgressEvent::ToolCompleted {
                                tool_name: tool_name.clone(),
                                tool_input: tool_input_for_progress.clone(),
                                success: false,
                                summary: output_summary,
                            });
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
            if let Some(ref cb) = self.progress_callback {
                cb(ProgressEvent::TokenCount(context.token_count));
            }

            // Budget re-check: ensure we haven't blown past the context window
            // during the tool loop. Tool results can be massive (file contents, grep, etc.)
            // and without this check we'd send 200K+ tokens and get billed for it.
            let usage_pct = context.usage_percentage();
            if usage_pct > 80.0 {
                tracing::warn!(
                    "Context at {:.0}% ({} tokens) inside tool loop — forcing compaction",
                    usage_pct,
                    context.token_count
                );
                match self.compact_context(&mut context, &model_name).await {
                    Ok(summary) => {
                        tracing::info!(
                            "Mid-loop compaction complete: {} tokens, summary len={}",
                            context.token_count,
                            summary.len()
                        );
                        // Inject continuation prompt so the LLM resumes the task
                        let mut cont_text = "[SYSTEM: Mid-loop context compaction complete. The summary above has \
                             full context of everything done so far. Briefly acknowledge the \
                             compaction to the user with a fun/cheeky remark (be creative, surprise \
                             them — cursing allowed), then pick up where you left off. Do NOT re-do \
                             completed work.]".to_string();
                        if !self.auto_approve_tools {
                            cont_text.push_str("\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user approval before EVERY tool execution. Do NOT batch tool calls without approval.");
                        }
                        context.add_message(Message::user(cont_text));
                    }
                    Err(e) => {
                        tracing::error!("Mid-loop compaction failed: {}", e);
                        loop_break_reason = Some(format!("Mid-loop compaction failed: {}", e));
                        break;
                    }
                }
            }

            // Hard safety net: force-truncate if still above 90% after compaction attempt
            {
                let pct = context.usage_percentage();
                if pct > 90.0 {
                    tracing::warn!("Context at {:.0}% post-tool-results — hard-truncating", pct);
                    let target = (context.max_tokens as f64 * 0.6) as usize;
                    context.trim_to_target(target);
                }
            }

            // Check for queued user messages to inject between tool iterations.
            // This lets the user provide follow-up feedback mid-execution (like Claude Code).
            if let Some(ref queue_cb) = self.message_queue_callback
                && let Some(queued_msg) = queue_cb().await
            {
                tracing::info!("Injecting queued user message between tool iterations");
                let injected = Message::user(queued_msg.clone());
                context.add_message(injected);

                // Save to database so conversation history stays consistent
                let _ = message_service
                    .create_message(session_id, "user".to_string(), queued_msg)
                    .await;
            }
        }

        let response = final_response.ok_or_else(|| {
            let reason = loop_break_reason
                .unwrap_or_else(|| "Tool loop ended without final response".to_string());
            AgentError::Internal(reason)
        })?;

        // Extract text from the final response only (for TUI display).
        // Intermediate text was already shown in real-time via IntermediateText events.
        let final_text = Self::extract_text_from_response(&response);

        // Save full accumulated text to database (preserves all intermediate messages for history)
        let assistant_db_msg = message_service
            .create_message(session_id, "assistant".to_string(), accumulated_text)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Calculate total cost
        let total_tokens = total_input_tokens + total_output_tokens;
        let cost =
            self.provider
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

        Ok(AgentResponse {
            message_id: assistant_db_msg.id,
            content: final_text,
            stop_reason: response.stop_reason,
            usage: crate::brain::provider::TokenUsage {
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
            },
            context_tokens: last_input_tokens,
            cost,
            model: response.model,
        })
    }

    /// Helper to prepare message context for LLM requests
    ///
    /// This extracts the common setup logic shared between send_message() and
    /// send_message_streaming() to reduce code duplication.
    async fn prepare_message_context(
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

        let model_name = model.unwrap_or_else(|| self.provider.default_model().to_string());
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

    /// Stream a request and accumulate into an LLMResponse.
    ///
    /// Sends text deltas to the progress callback as `StreamingChunk` events
    /// so the TUI can display them in real-time. Returns the full response
    /// once the stream completes, ready for tool extraction.
    async fn stream_complete(
        &self,
        request: LLMRequest,
        cancel_token: Option<&CancellationToken>,
    ) -> std::result::Result<LLMResponse, crate::brain::provider::ProviderError> {
        use crate::brain::provider::{ContentDelta, StreamEvent, TokenUsage};
        use futures::StreamExt;

        let request_model = request.model.clone();
        let mut stream = self.provider.stream(request).await?;

        // Accumulate state from stream events
        let mut id = String::new();
        let mut model = String::new();
        let mut stop_reason: Option<StopReason> = None;
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;

        // Track partial content blocks by index
        // Text blocks: accumulate text deltas
        // ToolUse blocks: accumulate JSON deltas
        struct BlockState {
            block: ContentBlock,
            json_buf: String, // for tool use JSON accumulation
        }
        let mut block_states: Vec<BlockState> = Vec::new();

        while let Some(event_result) = stream.next().await {
            // Check for cancellation between stream events
            if let Some(token) = cancel_token
                && token.is_cancelled()
            {
                tracing::info!("Stream cancelled by user");
                break;
            }
            let event = match event_result {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Stream error: {}", e);
                    return Err(e);
                }
            };

            match event {
                StreamEvent::MessageStart { message } => {
                    id = message.id;
                    model = message.model;
                    input_tokens = message.usage.input_tokens;
                }
                StreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => {
                    // Ensure block_states has enough capacity
                    while block_states.len() <= index {
                        block_states.push(BlockState {
                            block: ContentBlock::Text {
                                text: String::new(),
                            },
                            json_buf: String::new(),
                        });
                    }
                    block_states[index] = BlockState {
                        block: content_block,
                        json_buf: String::new(),
                    };
                }
                StreamEvent::ContentBlockDelta { index, delta } => {
                    if index < block_states.len() {
                        match delta {
                            ContentDelta::TextDelta { text } => {
                                // Forward to TUI for real-time display
                                if let Some(ref cb) = self.progress_callback {
                                    cb(ProgressEvent::StreamingChunk { text: text.clone() });
                                }
                                // Accumulate into block
                                if let ContentBlock::Text { text: ref mut t } =
                                    block_states[index].block
                                {
                                    t.push_str(&text);
                                }
                            }
                            ContentDelta::InputJsonDelta { partial_json } => {
                                block_states[index].json_buf.push_str(&partial_json);
                            }
                        }
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    if index < block_states.len() {
                        let state = &mut block_states[index];
                        // Finalize tool use blocks: parse accumulated JSON
                        if let ContentBlock::ToolUse { ref mut input, .. } = state.block
                            && !state.json_buf.is_empty()
                            && let Ok(parsed) = serde_json::from_str(&state.json_buf)
                        {
                            *input = parsed;
                        }
                    }
                }
                StreamEvent::MessageDelta { delta, usage } => {
                    stop_reason = delta.stop_reason;
                    // Take the largest values — MiniMax sends two deltas:
                    // first (0,0), then the real usage. Other providers
                    // may only send one. Using max() handles both cases.
                    if usage.input_tokens > input_tokens {
                        input_tokens = usage.input_tokens;
                    }
                    if usage.output_tokens > output_tokens {
                        output_tokens = usage.output_tokens;
                    }
                }
                StreamEvent::MessageStop => break,
                StreamEvent::Ping => {}
                StreamEvent::Error { error } => {
                    return Err(crate::brain::provider::ProviderError::StreamError(error));
                }
            }
        }

        // Detect premature stream termination — if we accumulated blocks but never
        // got a stop_reason, the connection likely dropped before [DONE]/MessageStop.
        if stop_reason.is_none() && !block_states.is_empty() {
            tracing::warn!(
                "⚠️ Stream ended without MessageStop/[DONE]. {} content blocks accumulated, \
                 {} output tokens counted. Possible network interruption or provider timeout.",
                block_states.len(),
                output_tokens,
            );
        }

        // Build final content blocks from accumulated state
        // Filter out empty text blocks — Anthropic rejects "text content blocks must be non-empty"
        let content_blocks: Vec<ContentBlock> = block_states
            .into_iter()
            .map(|s| s.block)
            .filter(|b| !matches!(b, ContentBlock::Text { text } if text.is_empty()))
            .collect();

        Ok(LLMResponse {
            id,
            // Some providers (e.g. MiniMax) don't include the model name in stream chunks.
            // Fall back to the request model so pricing lookup never gets an empty string.
            model: if model.is_empty() {
                request_model
            } else {
                model
            },
            content: content_blocks,
            stop_reason,
            usage: TokenUsage {
                input_tokens,
                output_tokens,
            },
        })
    }

    /// Trim DB messages to fit within the context budget.
    ///
    /// Keeps only the most recent messages that fit within ~60% of the context window
    /// after reserving space for tool definitions, brain, and response.
    /// Uses tiktoken cl100k_base for accurate token counting — no more chars/N guessing.
    fn trim_messages_to_budget(
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
    async fn compact_context(
        &self,
        context: &mut AgentContext,
        model_name: &str,
    ) -> Result<String> {
        // Emit compacting progress
        if let Some(ref cb) = self.progress_callback {
            cb(ProgressEvent::Compacting);
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
        let response = self
            .stream_complete(request, None)
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
            cb(ProgressEvent::CompactionSummary {
                summary: summary.clone(),
            });
        }

        Ok(summary)
    }

    /// Save a compaction summary to a daily memory log at `~/.opencrabs/memory/YYYY-MM-DD.md`.
    ///
    /// Multiple compactions per day append to the same file. The brain workspace's
    /// `MEMORY.md` is left untouched — it stays as user-curated durable memory.
    async fn save_to_memory(&self, summary: &str) -> std::result::Result<(), String> {
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

    /// Build a user Message, auto-attaching images from `<<IMG:path>>` markers.
    /// The TUI inserts these markers for detected image paths/URLs (handles spaces).
    async fn build_user_message(text: &str) -> Message {
        let mut image_blocks: Vec<ContentBlock> = Vec::new();

        // Extract <<IMG:path>> markers
        let mut clean_text = text.to_string();
        while let Some(start) = clean_text.find("<<IMG:") {
            if let Some(end) = clean_text[start..].find(">>") {
                let marker_end = start + end + 2;
                let img_path = &clean_text[start + 6..start + end];

                // URL image
                if img_path.starts_with("http://") || img_path.starts_with("https://") {
                    image_blocks.push(ContentBlock::Image {
                        source: ImageSource::Url {
                            url: img_path.to_string(),
                        },
                    });
                    tracing::info!("Auto-attached image URL: {}", img_path);
                }
                // Local file
                else {
                    let path = std::path::Path::new(img_path);
                    if let Ok(data) = tokio::fs::read(path).await {
                        let lower = img_path.to_lowercase();
                        let media_type = match lower.rsplit('.').next().unwrap_or("") {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "webp" => "image/webp",
                            "bmp" => "image/bmp",
                            "svg" => "image/svg+xml",
                            _ => "application/octet-stream",
                        };
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        image_blocks.push(ContentBlock::Image {
                            source: ImageSource::Base64 {
                                media_type: media_type.to_string(),
                                data: b64,
                            },
                        });
                        tracing::info!(
                            "Auto-attached image: {} ({}, {} bytes)",
                            img_path,
                            media_type,
                            data.len()
                        );
                    } else {
                        tracing::warn!("Could not read image file: {}", img_path);
                    }
                }

                // Remove marker from text
                clean_text = format!("{}{}", &clean_text[..start], &clean_text[marker_end..]);
            } else {
                break; // Malformed marker
            }
        }

        let clean_text = clean_text.trim().to_string();

        if image_blocks.is_empty() {
            Message::user(clean_text)
        } else {
            // Text first, then images
            let mut blocks = vec![ContentBlock::Text { text: clean_text }];
            blocks.extend(image_blocks);
            Message {
                role: Role::User,
                content: blocks,
            }
        }
    }

    /// Compact tool description for DB persistence (mirrors TUI's format_tool_description)
    fn format_tool_summary(tool_name: &str, tool_input: &Value) -> String {
        match tool_name {
            "bash" => {
                let cmd = tool_input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short: String = cmd.chars().take(60).collect();
                if cmd.len() > 60 {
                    format!("bash: {}…", short)
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
                let p = tool_input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Glob {}", p)
            }
            "grep" => {
                let p = tool_input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Grep '{}'", p)
            }
            "web_search" | "exa_search" | "brave_search" => {
                let q = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Search: {}", q)
            }
            "plan" => {
                let op = tool_input
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Plan: {}", op)
            }
            "task_manager" => {
                let op = tool_input
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Task: {}", op)
            }
            "memory_search" => {
                let q = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Memory: {}", q)
            }
            other => other.to_string(),
        }
    }

    /// Extract text content from an LLM response (text blocks only — tool calls
    /// are displayed via the tool group UI, not as raw text).
    fn extract_text_from_response(response: &LLMResponse) -> String {
        let mut text = String::new();

        for content in &response.content {
            if let ContentBlock::Text { text: t } = content
                && !t.trim().is_empty()
            {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(t);
            }
        }

        text
    }
}

/// Response from the agent
#[derive(Debug, Clone)]
pub struct AgentResponse {
    /// Message ID in database
    pub message_id: Uuid,

    /// Response content
    pub content: String,

    /// Stop reason
    pub stop_reason: Option<StopReason>,

    /// Token usage (accumulated across all tool-loop iterations — for billing)
    pub usage: crate::brain::provider::TokenUsage,

    /// Actual context window usage from the last API call (for display)
    pub context_tokens: u32,

    /// Cost in USD
    pub cost: f64,

    /// Model used
    pub model: String,
}

/// Streaming response from the agent
pub struct AgentStreamResponse {
    /// Session ID
    pub session_id: Uuid,

    /// Message ID that will be created
    pub message_id: Uuid,

    /// Stream of events
    pub stream: ProviderStream,

    /// Model being used
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::provider::{LLMRequest, LLMResponse, TokenUsage};
    use crate::db::Database;
    use async_trait::async_trait;

    /// Mock provider for testing
    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(
            &self,
            _request: LLMRequest,
        ) -> crate::brain::provider::Result<LLMResponse> {
            Ok(LLMResponse {
                id: "test-response-1".to_string(),
                model: "mock-model".to_string(),
                content: vec![ContentBlock::Text {
                    text: "This is a test response".to_string(),
                }],
                stop_reason: Some(StopReason::EndTurn),
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                },
            })
        }

        async fn stream(
            &self,
            request: LLMRequest,
        ) -> crate::brain::provider::Result<ProviderStream> {
            use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

            let response = self.complete(request).await?;
            let mut events = vec![Ok(StreamEvent::MessageStart {
                message: StreamMessage {
                    id: response.id.clone(),
                    model: response.model.clone(),
                    role: Role::Assistant,
                    usage: response.usage,
                },
            })];
            for (i, block) in response.content.iter().enumerate() {
                if let ContentBlock::Text { text } = block {
                    events.push(Ok(StreamEvent::ContentBlockStart {
                        index: i,
                        content_block: ContentBlock::Text {
                            text: String::new(),
                        },
                    }));
                    events.push(Ok(StreamEvent::ContentBlockDelta {
                        index: i,
                        delta: ContentDelta::TextDelta { text: text.clone() },
                    }));
                    events.push(Ok(StreamEvent::ContentBlockStop { index: i }));
                }
            }
            events.push(Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: response.stop_reason,
                    stop_sequence: None,
                },
                usage: response.usage,
            }));
            events.push(Ok(StreamEvent::MessageStop));
            Ok(Box::pin(futures::stream::iter(events)))
        }

        fn name(&self) -> &str {
            "mock"
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        fn supported_models(&self) -> Vec<String> {
            vec!["mock-model".to_string()]
        }

        fn context_window(&self, _model: &str) -> Option<u32> {
            Some(4096)
        }

        fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
            0.001 // Mock cost
        }
    }

    async fn create_test_service() -> (AgentService, Uuid) {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool().clone();

        let context = ServiceContext::new(pool);
        let provider = Arc::new(MockProvider);

        let agent_service = AgentService::new(provider, context.clone());

        // Create a test session
        let session_service = SessionService::new(context);
        let session = session_service
            .create_session(Some("Test Session".to_string()))
            .await
            .unwrap();

        (agent_service, session.id)
    }

    #[tokio::test]
    async fn test_agent_service_creation() {
        let (agent_service, _) = create_test_service().await;
        assert_eq!(agent_service.max_tool_iterations, 0); // 0 = unlimited
    }

    #[tokio::test]
    async fn test_send_message() {
        let (agent_service, session_id) = create_test_service().await;

        let response = agent_service
            .send_message(session_id, "Hello, world!".to_string(), None)
            .await
            .unwrap();

        assert!(!response.content.is_empty());
        assert_eq!(response.model, "mock-model");
        assert!(response.cost > 0.0);
    }

    #[tokio::test]
    async fn test_send_message_with_system_brain() {
        let (agent_service, session_id) = create_test_service().await;

        let agent_service =
            agent_service.with_system_brain("You are a helpful assistant.".to_string());

        let response = agent_service
            .send_message(session_id, "Hello!".to_string(), None)
            .await
            .unwrap();

        assert!(!response.content.is_empty());
    }

    /// Mock provider that simulates tool use
    struct MockProviderWithTools {
        call_count: std::sync::Mutex<usize>,
    }

    impl MockProviderWithTools {
        fn new() -> Self {
            Self {
                call_count: std::sync::Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl Provider for MockProviderWithTools {
        async fn complete(
            &self,
            _request: LLMRequest,
        ) -> crate::brain::provider::Result<LLMResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let call_num = *count;

            if call_num == 1 {
                // First call: request tool use
                Ok(LLMResponse {
                    id: "test-response-1".to_string(),
                    model: "mock-model".to_string(),
                    content: vec![
                        ContentBlock::Text {
                            text: "I'll use the test tool.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "tool-1".to_string(),
                            name: "test_tool".to_string(),
                            input: serde_json::json!({"message": "test"}),
                        },
                    ],
                    stop_reason: Some(StopReason::ToolUse),
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 20,
                    },
                })
            } else {
                // Second call: final response after tool execution
                Ok(LLMResponse {
                    id: "test-response-2".to_string(),
                    model: "mock-model".to_string(),
                    content: vec![ContentBlock::Text {
                        text: "Tool execution completed successfully.".to_string(),
                    }],
                    stop_reason: Some(StopReason::EndTurn),
                    usage: TokenUsage {
                        input_tokens: 15,
                        output_tokens: 25,
                    },
                })
            }
        }

        async fn stream(
            &self,
            request: LLMRequest,
        ) -> crate::brain::provider::Result<ProviderStream> {
            use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

            // Get the response that complete() would return, then convert to stream events
            let response = self.complete(request).await?;
            let mut events = vec![Ok(StreamEvent::MessageStart {
                message: StreamMessage {
                    id: response.id.clone(),
                    model: response.model.clone(),
                    role: Role::Assistant,
                    usage: response.usage,
                },
            })];

            for (i, block) in response.content.iter().enumerate() {
                // ContentBlockStart sends empty shells; actual content comes via deltas
                match block {
                    ContentBlock::Text { text } => {
                        events.push(Ok(StreamEvent::ContentBlockStart {
                            index: i,
                            content_block: ContentBlock::Text {
                                text: String::new(),
                            },
                        }));
                        events.push(Ok(StreamEvent::ContentBlockDelta {
                            index: i,
                            delta: ContentDelta::TextDelta { text: text.clone() },
                        }));
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        events.push(Ok(StreamEvent::ContentBlockStart {
                            index: i,
                            content_block: ContentBlock::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input: serde_json::Value::Object(Default::default()),
                            },
                        }));
                        events.push(Ok(StreamEvent::ContentBlockDelta {
                            index: i,
                            delta: ContentDelta::InputJsonDelta {
                                partial_json: serde_json::to_string(input).unwrap_or_default(),
                            },
                        }));
                    }
                    _ => {
                        events.push(Ok(StreamEvent::ContentBlockStart {
                            index: i,
                            content_block: block.clone(),
                        }));
                    }
                }
                events.push(Ok(StreamEvent::ContentBlockStop { index: i }));
            }

            events.push(Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: response.stop_reason,
                    stop_sequence: None,
                },
                usage: response.usage,
            }));
            events.push(Ok(StreamEvent::MessageStop));

            Ok(Box::pin(futures::stream::iter(events)))
        }

        fn name(&self) -> &str {
            "mock-with-tools"
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        fn supported_models(&self) -> Vec<String> {
            vec!["mock-model".to_string()]
        }

        fn context_window(&self, _model: &str) -> Option<u32> {
            Some(4096)
        }

        fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
            0.001
        }
    }

    /// Mock tool for testing
    struct MockTool;

    #[async_trait]
    impl crate::brain::tools::Tool for MockTool {
        fn name(&self) -> &str {
            "test_tool"
        }

        fn description(&self) -> &str {
            "A test tool"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            })
        }

        fn capabilities(&self) -> Vec<crate::brain::tools::ToolCapability> {
            vec![]
        }

        fn requires_approval(&self) -> bool {
            false
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _context: &crate::brain::tools::ToolExecutionContext,
        ) -> crate::brain::tools::Result<crate::brain::tools::ToolResult> {
            Ok(crate::brain::tools::ToolResult::success(
                "Tool executed successfully".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn test_send_message_with_tool_execution() {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool().clone();

        let context = ServiceContext::new(pool);
        let provider = Arc::new(MockProviderWithTools::new());

        // Create tool registry and register our test tool
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(MockTool));

        let agent_service = AgentService::new(provider, context.clone())
            .with_tool_registry(Arc::new(registry))
            .with_auto_approve_tools(true);

        // Create a test session
        let session_service = SessionService::new(context);
        let session = session_service
            .create_session(Some("Test Session".to_string()))
            .await
            .unwrap();

        // Send message with tool execution
        let response = agent_service
            .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
            .await
            .unwrap();

        assert!(!response.content.is_empty());
        assert!(response.content.contains("completed successfully"));
        assert_eq!(response.model, "mock-model");
        // Should have tokens from both calls
        assert!(response.usage.input_tokens >= 25); // 10 + 15
        assert!(response.usage.output_tokens >= 45); // 20 + 25
    }

    #[tokio::test]
    async fn test_message_queue_injection_between_tool_calls() {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool().clone();

        let context = ServiceContext::new(pool);
        let provider = Arc::new(MockProviderWithTools::new());

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(MockTool));

        // Set up a message queue with a queued message
        let queue: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(Some("user follow-up".to_string())));

        let queue_clone = queue.clone();
        let message_queue_callback: MessageQueueCallback = Arc::new(move || {
            let q = queue_clone.clone();
            Box::pin(async move { q.lock().await.take() })
        });

        let agent_service = AgentService::new(provider, context.clone())
            .with_tool_registry(Arc::new(registry))
            .with_auto_approve_tools(true)
            .with_message_queue_callback(Some(message_queue_callback));

        let session_service = SessionService::new(context.clone());
        let session = session_service
            .create_session(Some("Queue Test".to_string()))
            .await
            .unwrap();

        // Send message — the mock provider will do a tool call on first LLM call,
        // then the queue callback will inject "user follow-up" between iterations
        let response = agent_service
            .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
            .await
            .unwrap();

        assert!(!response.content.is_empty());

        // Verify the queue was drained
        assert!(queue.lock().await.is_none());

        // Verify the injected message was saved to database
        let message_service = MessageService::new(context);
        let messages = message_service
            .list_messages_for_session(session.id)
            .await
            .unwrap();

        let user_messages: Vec<_> = messages.iter().filter(|m| m.role == "user").collect();

        // Should have original message + injected follow-up
        assert!(
            user_messages.len() >= 2,
            "expected at least 2 user messages (original + injected), got {}",
            user_messages.len()
        );

        let has_followup = user_messages.iter().any(|m| m.content == "user follow-up");
        assert!(
            has_followup,
            "injected follow-up message not found in database"
        );
    }

    #[tokio::test]
    async fn test_message_queue_empty_no_injection() {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool().clone();

        let context = ServiceContext::new(pool);
        let provider = Arc::new(MockProviderWithTools::new());

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(MockTool));

        // Empty queue — should not inject anything
        let queue: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let queue_clone = queue.clone();
        let message_queue_callback: MessageQueueCallback = Arc::new(move || {
            let q = queue_clone.clone();
            Box::pin(async move { q.lock().await.take() })
        });

        let agent_service = AgentService::new(provider, context.clone())
            .with_tool_registry(Arc::new(registry))
            .with_auto_approve_tools(true)
            .with_message_queue_callback(Some(message_queue_callback));

        let session_service = SessionService::new(context.clone());
        let session = session_service
            .create_session(Some("Empty Queue Test".to_string()))
            .await
            .unwrap();

        let response = agent_service
            .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
            .await
            .unwrap();

        assert!(!response.content.is_empty());

        // Only 1 user message (the original), no injected messages
        let message_service = MessageService::new(context);
        let messages = message_service
            .list_messages_for_session(session.id)
            .await
            .unwrap();

        let user_messages: Vec<_> = messages.iter().filter(|m| m.role == "user").collect();

        assert_eq!(
            user_messages.len(),
            1,
            "should only have original user message"
        );
    }

    #[tokio::test]
    async fn test_stream_complete_text_only() {
        // Verify stream_complete reconstructs a text-only response correctly
        let (agent_service, _) = create_test_service().await;

        let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);

        let response = agent_service.stream_complete(request, None).await.unwrap();
        assert_eq!(response.model, "mock-model");
        assert!(!response.content.is_empty());

        // Should have a text block
        let has_text = response
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if !text.is_empty()));
        assert!(has_text, "response should contain non-empty text");
        assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
        assert!(response.usage.input_tokens > 0 || response.usage.output_tokens > 0);
    }

    #[tokio::test]
    async fn test_stream_complete_with_tool_use() {
        // Verify stream_complete reconstructs tool use blocks from stream events
        let provider = Arc::new(MockProviderWithTools::new());
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let context = ServiceContext::new(db.pool().clone());
        let agent_service = AgentService::new(provider, context);

        let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Use a tool")]);

        let response = agent_service.stream_complete(request, None).await.unwrap();

        // First call to MockProviderWithTools returns text + tool_use
        let text_blocks: Vec<_> = response
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::Text { .. }))
            .collect();
        let tool_blocks: Vec<_> = response
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .collect();

        assert!(!text_blocks.is_empty(), "should have text block");
        assert!(!tool_blocks.is_empty(), "should have tool_use block");
        assert_eq!(response.stop_reason, Some(StopReason::ToolUse));

        // Verify tool use has correct name and parsed input
        if let ContentBlock::ToolUse { name, input, .. } = &tool_blocks[0] {
            assert_eq!(name, "test_tool");
            assert_eq!(input.get("message").and_then(|v| v.as_str()), Some("test"));
        }
    }

    #[tokio::test]
    async fn test_streaming_chunks_emitted() {
        // Verify StreamingChunk progress events are emitted during streaming
        use std::sync::Mutex;

        let provider = Arc::new(MockProvider);
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let context = ServiceContext::new(db.pool().clone());

        let chunks_received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let chunks_clone = chunks_received.clone();

        let progress_cb: ProgressCallback = Arc::new(move |event| {
            if let ProgressEvent::StreamingChunk { text } = event {
                chunks_clone.lock().unwrap().push(text);
            }
        });

        let agent_service =
            AgentService::new(provider, context).with_progress_callback(Some(progress_cb));

        let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);

        let _response = agent_service.stream_complete(request, None).await.unwrap();

        let chunks = chunks_received.lock().unwrap();
        assert!(!chunks.is_empty(), "should have received streaming chunks");
        let combined: String = chunks.iter().cloned().collect();
        assert!(!combined.is_empty(), "combined chunks should have content");
    }

    #[tokio::test]
    async fn test_context_tokens_is_last_iteration_not_accumulated() {
        // When tool loop runs 2 iterations (10 + 15 input tokens),
        // context_tokens should be 15 (last iteration), not 25 (sum).
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let context = ServiceContext::new(db.pool().clone());
        let provider = Arc::new(MockProviderWithTools::new());

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(MockTool));

        let agent_service = AgentService::new(provider, context.clone())
            .with_tool_registry(Arc::new(registry))
            .with_auto_approve_tools(true);

        let session_service = SessionService::new(context);
        let session = session_service
            .create_session(Some("Context Tokens Test".to_string()))
            .await
            .unwrap();

        let response = agent_service
            .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
            .await
            .unwrap();

        // usage.input_tokens = accumulated (10 + 15 = 25) — for billing
        assert_eq!(response.usage.input_tokens, 25);
        // context_tokens = last iteration only (15) — for display
        assert_eq!(response.context_tokens, 15);
    }

    #[tokio::test]
    async fn test_context_tokens_equals_input_tokens_without_tools() {
        // Without tool use, context_tokens should equal usage.input_tokens
        // (single API call, no accumulation).
        let (agent_service, session_id) = create_test_service().await;

        let response = agent_service
            .send_message(session_id, "Hello".to_string(), None)
            .await
            .unwrap();

        assert_eq!(response.context_tokens, response.usage.input_tokens);
        assert_eq!(response.context_tokens, 10); // MockProvider returns 10
    }
}
