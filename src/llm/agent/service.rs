//! Agent Service Implementation
//!
//! Core service for managing AI agent conversations, coordinating between
//! LLM providers, context management, and data persistence.

use super::context::AgentContext;
use super::error::{AgentError, Result};
use crate::llm::provider::{
    ContentBlock, ImageSource, LLMRequest, LLMResponse, Message, Provider, ProviderStream, Role,
    StopReason,
};
use crate::llm::tools::{ToolExecutionContext, ToolRegistry};
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
    ToolStarted { tool_name: String, tool_input: Value },
    ToolCompleted { tool_name: String, tool_input: Value, success: bool, summary: String },
    /// Intermediate text the agent sends between tool call batches
    IntermediateText { text: String },
    /// Real-time streaming chunk from the LLM (word-by-word)
    StreamingChunk { text: String },
    Compacting,
    /// Compaction finished — carry the summary so the TUI can display it
    CompactionSummary { summary: String },
}

/// Callback for reporting progress during agent execution
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// Callback for checking if a user message has been queued during tool execution.
/// Returns Some(message) if a message is waiting, None otherwise. Must not block.
pub type MessageQueueCallback = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync,
>;

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

    /// Callback for requesting tool approval from user
    approval_callback: Option<ApprovalCallback>,

    /// Callback for reporting progress during tool execution
    progress_callback: Option<ProgressCallback>,

    /// Callback for checking queued user messages between tool iterations
    message_queue_callback: Option<MessageQueueCallback>,

    /// Working directory for tool execution
    working_directory: std::path::PathBuf,

    /// Brain path (~/.opencrabs/) for loading brain files
    brain_path: Option<std::path::PathBuf>,
}

impl AgentService {
    /// Create a new agent service
    pub fn new(provider: Arc<dyn Provider>, context: ServiceContext) -> Self {
        Self {
            provider,
            context,
            tool_registry: Arc::new(ToolRegistry::new()),
            max_tool_iterations: 0, // 0 = unlimited (loop detection is the safety net)
            default_system_brain: None,
            auto_approve_tools: false,
            approval_callback: None,
            progress_callback: None,
            message_queue_callback: None,
            working_directory: std::env::current_dir().unwrap_or_default(),
            brain_path: None,
        }
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

    /// Set the working directory for tool execution
    pub fn with_working_directory(mut self, working_directory: std::path::PathBuf) -> Self {
        self.working_directory = working_directory;
        self
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

    /// Get the default model for this provider
    pub fn provider_model(&self) -> &str {
        self.provider.default_model()
    }

    /// Get the list of supported models for this provider
    pub fn supported_models(&self) -> Vec<String> {
        self.provider.supported_models()
    }

    /// Get a reference to the underlying LLM provider
    pub fn provider(&self) -> &Arc<dyn Provider> {
        &self.provider
    }

    /// Get context window size for a given model
    pub fn context_window_for_model(&self, model: &str) -> u32 {
        self.provider.context_window(model).unwrap_or(200_000)
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
        let context_window = self.provider.context_window(&model_name).unwrap_or(4096);

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
            let _ = self.compact_context(&mut context, &model_name).await;
        }

        // Create tool execution context
        let tool_context = ToolExecutionContext::new(session_id)
            .with_auto_approve(self.auto_approve_tools)
            .with_working_directory(self.working_directory.clone())
            .with_read_only_mode(read_only_mode);

        // Tool execution loop
        let mut iteration = 0;
        let mut total_input_tokens = 0u32;
        let mut total_output_tokens = 0u32;
        let mut final_response: Option<LLMResponse> = None;
        let mut accumulated_text = String::new(); // Collect text from all iterations (not just final)
        let mut recent_tool_calls: Vec<String> = Vec::new(); // Track tool calls to detect loops

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
                && token.is_cancelled() {
                    break;
                }

            iteration += 1;

            // Emit thinking progress
            if let Some(ref cb) = self.progress_callback {
                cb(ProgressEvent::Thinking);
            }

            // Build LLM request with tools if available
            let mut request =
                LLMRequest::new(model_name.clone(), context.messages.clone()).with_max_tokens(4096);

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
                Err(ref e) if e.to_string().contains("prompt is too long") || e.to_string().contains("too many tokens") => {
                    tracing::warn!("Prompt too long for provider — emergency compaction");
                    let _ = self.compact_context(&mut context, &model_name).await;

                    // Rebuild request with compacted context
                    let mut retry_req = LLMRequest::new(model_name.clone(), context.messages.clone())
                        .with_max_tokens(4096);
                    if let Some(system) = &context.system_brain {
                        retry_req = retry_req.with_system(system.clone());
                    }
                    if self.tool_registry.count() > 0 {
                        retry_req = retry_req.with_tools(self.tool_registry.get_tool_definitions());
                    }
                    self.stream_complete(retry_req, cancel_token.as_ref()).await.map_err(AgentError::Provider)?
                }
                Err(e) => return Err(AgentError::Provider(e)),
            };

            // Track token usage
            total_input_tokens += response.usage.input_tokens;
            total_output_tokens += response.usage.output_tokens;

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
                        tracing::debug!("Block {}: ToolUse {{ name: {}, id: {} }}", i, name, id);
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
                // No tool use - we're done
                tracing::debug!("No tool uses found, completing with final response");
                final_response = Some(response);
                break;
            }

            // Emit intermediate text to TUI so it appears before the tool calls
            if !iteration_text.is_empty()
                && let Some(ref cb) = self.progress_callback {
                    cb(ProgressEvent::IntermediateText { text: iteration_text });
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

                        // Other tools: just use name
                        _ => name.to_string(),
                    }
                })
                .collect::<Vec<_>>()
                .join(",");

            recent_tool_calls.push(current_call_signature.clone());

            // Keep only last 15 iterations for loop detection (increased for deep exploration)
            if recent_tool_calls.len() > 15 {
                recent_tool_calls.remove(0);
            }

            // Check for repeated patterns with tool-specific thresholds
            // This will only trigger for truly identical calls (same tool + same arguments)

            // Determine loop threshold based on tool type
            let is_exploration_tool = current_call_signature.starts_with("ls:")
                || current_call_signature.starts_with("glob:")
                || current_call_signature.starts_with("grep:")
                || current_call_signature.starts_with("read:");

            let is_modification_tool = current_call_signature.starts_with("write:")
                || current_call_signature.starts_with("edit:")
                || current_call_signature.starts_with("bash:");

            // Higher threshold for exploration tools (allow deep directory traversal)
            // Lower threshold for modification tools (dangerous if looping)
            let loop_threshold = if is_exploration_tool {
                10 // Allow up to 10 identical calls for exploration
            } else if is_modification_tool {
                2 // Only 2 identical calls for modification tools
            } else {
                3 // Default: 3 identical calls
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

                    if is_exploration_tool {
                        tracing::info!(
                            "💡 Hint: The model is stuck trying to access the same path {} times. \
                             This often means the path doesn't exist or the model is confused about the directory structure.",
                            loop_threshold
                        );
                    } else if is_modification_tool {
                        tracing::warn!(
                            "⚠️ Modification tool loop detected! This could be dangerous. \
                             The model tried to modify the same file/run the same command {} times.",
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

            for (tool_id, tool_name, tool_input) in tool_uses {
                // Check for cancellation before each tool
                if let Some(ref token) = cancel_token
                    && token.is_cancelled() {
                        break;
                    }

                tracing::info!(
                    "Executing tool '{}' (iteration {})",
                    tool_name,
                    iteration,
                );

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
                            tool_results.push(ContentBlock::ToolResult {
                                tool_use_id: tool_id,
                                content: format!("Tool not found: {}", tool_name),
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
                                        if let Some(ref cb) = self.progress_callback {
                                            cb(ProgressEvent::ToolCompleted {
                                                tool_name: tool_name.clone(),
                                                tool_input: tool_input_for_progress.clone(),
                                                success,
                                                summary: content.chars().take(100).collect(),
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
                                        if let Some(ref cb) = self.progress_callback {
                                            cb(ProgressEvent::ToolCompleted {
                                                tool_name: tool_name.clone(),
                                                tool_input: tool_input_for_progress.clone(),
                                                success: false,
                                                summary: err_msg.chars().take(100).collect(),
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
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: tool_id,
                            content: "Tool requires approval but no approval mechanism configured"
                                .to_string(),
                            is_error: Some(true),
                        });
                        continue;
                    }
                }

                // Execute the tool
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
                        if let Some(ref cb) = self.progress_callback {
                            cb(ProgressEvent::ToolCompleted {
                                tool_name: tool_name.clone(),
                                tool_input: tool_input_for_progress.clone(),
                                success,
                                summary: content.chars().take(100).collect(),
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
                        if let Some(ref cb) = self.progress_callback {
                            cb(ProgressEvent::ToolCompleted {
                                tool_name: tool_name.clone(),
                                tool_input: tool_input_for_progress.clone(),
                                success: false,
                                summary: err_msg.chars().take(100).collect(),
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

            // Append tool call summaries to accumulated text for DB persistence.
            // Format: <!-- tools: Read /foo.rs | Edit /bar.rs -->
            // This lets the session loader reconstruct tool groups on reload.
            if !tool_descriptions.is_empty() {
                if !accumulated_text.is_empty() {
                    accumulated_text.push('\n');
                }
                accumulated_text.push_str(&format!(
                    "<!-- tools: {} -->",
                    tool_descriptions.join(" | ")
                ));
                tool_descriptions.clear();
            }

            // Add assistant message with tool use to context (filter empty text blocks)
            let clean_content: Vec<ContentBlock> = response.content.iter()
                .filter(|b| !matches!(b, ContentBlock::Text { text } if text.is_empty()))
                .cloned()
                .collect();
            let assistant_msg = Message {
                role: crate::llm::provider::Role::Assistant,
                content: clean_content,
            };
            context.add_message(assistant_msg);

            // Add user message with tool results to context
            let tool_result_msg = Message {
                role: crate::llm::provider::Role::User,
                content: tool_results,
            };
            context.add_message(tool_result_msg);

            // Check for queued user messages to inject between tool iterations.
            // This lets the user provide follow-up feedback mid-execution (like Claude Code).
            if let Some(ref queue_cb) = self.message_queue_callback
                && let Some(queued_msg) = queue_cb().await {
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
            AgentError::Internal("Tool loop completed without final response".to_string())
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
            usage: crate::llm::provider::TokenUsage {
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
            },
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
        let context_window = self.provider.context_window(&model_name).unwrap_or(4096);

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
        let request =
            LLMRequest::new(model_name.clone(), context.messages.clone()).with_max_tokens(4096);

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
    async fn stream_complete(&self, request: LLMRequest, cancel_token: Option<&CancellationToken>) -> std::result::Result<LLMResponse, crate::llm::provider::ProviderError> {
        use crate::llm::provider::{ContentDelta, StreamEvent, TokenUsage};
        use futures::StreamExt;

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
            if let Some(token) = cancel_token && token.is_cancelled() {
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
                StreamEvent::ContentBlockStart { index, content_block } => {
                    // Ensure block_states has enough capacity
                    while block_states.len() <= index {
                        block_states.push(BlockState {
                            block: ContentBlock::Text { text: String::new() },
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
                                if let ContentBlock::Text { text: ref mut t } = block_states[index].block {
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
                            && let Ok(parsed) = serde_json::from_str(&state.json_buf) {
                                *input = parsed;
                        }
                    }
                }
                StreamEvent::MessageDelta { delta, usage } => {
                    stop_reason = delta.stop_reason;
                    output_tokens = usage.output_tokens;
                }
                StreamEvent::MessageStop => break,
                StreamEvent::Ping => {}
                StreamEvent::Error { error } => {
                    return Err(crate::llm::provider::ProviderError::StreamError(error));
                }
            }
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
            model,
            content: content_blocks,
            stop_reason,
            usage: TokenUsage { input_tokens, output_tokens },
        })
    }

    /// Trim DB messages to fit within the context budget.
    ///
    /// Keeps only the most recent messages that fit within ~70% of the context window
    /// after reserving space for tool definitions, brain, and response.
    fn trim_messages_to_budget(
        all_messages: Vec<crate::db::models::Message>,
        context_window: usize,
        tool_count: usize,
        brain: Option<&str>,
    ) -> Vec<crate::db::models::Message> {
        let tool_budget = tool_count * 500;
        let brain_budget = brain.map(|b| b.len() / 3).unwrap_or(0);
        let history_budget = context_window
            .saturating_sub(tool_budget)
            .saturating_sub(brain_budget)
            .saturating_sub(16384) // reserve for response
            * 70 / 100;

        let mut token_acc = 0usize;
        let mut keep_from = 0usize;
        for (i, msg) in all_messages.iter().enumerate().rev() {
            if msg.content.is_empty() {
                continue;
            }
            let msg_tokens = msg.content.len() / 3;
            if token_acc + msg_tokens > history_budget {
                keep_from = i + 1;
                break;
            }
            token_acc += msg_tokens;
        }

        if keep_from > 0 {
            let kept = all_messages.len() - keep_from;
            tracing::info!(
                "Context budget: keeping last {} of {} messages ({} est. tokens, budget {})",
                kept, all_messages.len(), token_acc, history_budget
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

        // Include all conversation messages so the LLM sees the full context
        for msg in &context.messages {
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
             Be concise but complete — this summary is the ONLY context the agent will have after compaction.",
            context.usage_percentage(),
            context.token_count,
            context.max_tokens,
            remaining_budget,
        );

        summary_messages.push(Message::user(compaction_prompt));

        let request = LLMRequest::new(model_name.to_string(), summary_messages)
            .with_max_tokens(4096)
            .with_system("You are a precise summarization assistant. Your job is to create a structured breakdown of the conversation that will serve as the complete context for an AI agent continuing this work after context compaction. Be thorough — include every file, decision, and pending task.".to_string());

        let response = self
            .provider
            .complete(request)
            .await
            .map_err(AgentError::Provider)?;

        let summary = Self::extract_text_from_response(&response);

        // Save to daily memory log
        if let Err(e) = self.save_to_memory(&summary).await {
            tracing::warn!("Failed to save compaction summary to daily log: {}", e);
        }

        // Trigger background re-index so memory_search picks up the new log
        crate::memory::reindex_background();

        // Compact the context: keep last 4 message pairs (8 messages)
        context.compact_with_summary(summary.clone(), 8);

        tracing::info!(
            "Context compacted: now at {:.0}% ({} tokens)",
            context.usage_percentage(),
            context.token_count
        );

        // Show the summary to the user in chat
        if let Some(ref cb) = self.progress_callback {
            cb(ProgressEvent::CompactionSummary { summary: summary.clone() });
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
                        source: ImageSource::Url { url: img_path.to_string() },
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
                        tracing::info!("Auto-attached image: {} ({}, {} bytes)", img_path, media_type, data.len());
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
                let cmd = tool_input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                let short: String = cmd.chars().take(60).collect();
                if cmd.len() > 60 { format!("bash: {}…", short) } else { format!("bash: {}", short) }
            }
            "read_file" | "read" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Read {}", path)
            }
            "write_file" | "write" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Write {}", path)
            }
            "edit_file" | "edit" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Edit {}", path)
            }
            "ls" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                format!("ls {}", path)
            }
            "glob" => {
                let p = tool_input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Glob {}", p)
            }
            "grep" => {
                let p = tool_input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Grep '{}'", p)
            }
            "web_search" | "exa_search" | "brave_search" => {
                let q = tool_input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Search: {}", q)
            }
            "plan" => {
                let op = tool_input.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Plan: {}", op)
            }
            "task_manager" => {
                let op = tool_input.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Task: {}", op)
            }
            "memory_search" => {
                let q = tool_input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
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

    /// Token usage
    pub usage: crate::llm::provider::TokenUsage,

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
    use crate::db::Database;
    use crate::llm::provider::{LLMRequest, LLMResponse, TokenUsage};
    use async_trait::async_trait;

    /// Mock provider for testing
    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(
            &self,
            _request: LLMRequest,
        ) -> crate::llm::provider::Result<LLMResponse> {
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
        ) -> crate::llm::provider::Result<ProviderStream> {
            use crate::llm::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

            let response = self.complete(request).await?;
            let mut events = vec![
                Ok(StreamEvent::MessageStart {
                    message: StreamMessage {
                        id: response.id.clone(),
                        model: response.model.clone(),
                        role: Role::Assistant,
                        usage: response.usage,
                    },
                }),
            ];
            for (i, block) in response.content.iter().enumerate() {
                if let ContentBlock::Text { text } = block {
                    events.push(Ok(StreamEvent::ContentBlockStart {
                        index: i,
                        content_block: ContentBlock::Text { text: String::new() },
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
        ) -> crate::llm::provider::Result<LLMResponse> {
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
        ) -> crate::llm::provider::Result<ProviderStream> {
            use crate::llm::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

            // Get the response that complete() would return, then convert to stream events
            let response = self.complete(request).await?;
            let mut events = vec![
                Ok(StreamEvent::MessageStart {
                    message: StreamMessage {
                        id: response.id.clone(),
                        model: response.model.clone(),
                        role: Role::Assistant,
                        usage: response.usage,
                    },
                }),
            ];

            for (i, block) in response.content.iter().enumerate() {
                // ContentBlockStart sends empty shells; actual content comes via deltas
                match block {
                    ContentBlock::Text { text } => {
                        events.push(Ok(StreamEvent::ContentBlockStart {
                            index: i,
                            content_block: ContentBlock::Text { text: String::new() },
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
    impl crate::llm::tools::Tool for MockTool {
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

        fn capabilities(&self) -> Vec<crate::llm::tools::ToolCapability> {
            vec![]
        }

        fn requires_approval(&self) -> bool {
            false
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _context: &crate::llm::tools::ToolExecutionContext,
        ) -> crate::llm::tools::Result<crate::llm::tools::ToolResult> {
            Ok(crate::llm::tools::ToolResult::success(
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

        let user_messages: Vec<_> = messages
            .iter()
            .filter(|m| m.role == "user")
            .collect();

        // Should have original message + injected follow-up
        assert!(
            user_messages.len() >= 2,
            "expected at least 2 user messages (original + injected), got {}",
            user_messages.len()
        );

        let has_followup = user_messages.iter().any(|m| m.content == "user follow-up");
        assert!(has_followup, "injected follow-up message not found in database");
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

        let user_messages: Vec<_> = messages
            .iter()
            .filter(|m| m.role == "user")
            .collect();

        assert_eq!(user_messages.len(), 1, "should only have original user message");
    }

    #[tokio::test]
    async fn test_stream_complete_text_only() {
        // Verify stream_complete reconstructs a text-only response correctly
        let (agent_service, _) = create_test_service().await;

        let request = LLMRequest::new(
            "mock-model".to_string(),
            vec![Message::user("Hello")],
        );

        let response = agent_service.stream_complete(request, None).await.unwrap();
        assert_eq!(response.model, "mock-model");
        assert!(!response.content.is_empty());

        // Should have a text block
        let has_text = response.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if !text.is_empty()));
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

        let request = LLMRequest::new(
            "mock-model".to_string(),
            vec![Message::user("Use a tool")],
        );

        let response = agent_service.stream_complete(request, None).await.unwrap();

        // First call to MockProviderWithTools returns text + tool_use
        let text_blocks: Vec<_> = response.content.iter().filter(|b| matches!(b, ContentBlock::Text { .. })).collect();
        let tool_blocks: Vec<_> = response.content.iter().filter(|b| matches!(b, ContentBlock::ToolUse { .. })).collect();

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

        let agent_service = AgentService::new(provider, context)
            .with_progress_callback(Some(progress_cb));

        let request = LLMRequest::new(
            "mock-model".to_string(),
            vec![Message::user("Hello")],
        );

        let _response = agent_service.stream_complete(request, None).await.unwrap();

        let chunks = chunks_received.lock().unwrap();
        assert!(!chunks.is_empty(), "should have received streaming chunks");
        let combined: String = chunks.iter().cloned().collect();
        assert!(!combined.is_empty(), "combined chunks should have content");
    }
}
